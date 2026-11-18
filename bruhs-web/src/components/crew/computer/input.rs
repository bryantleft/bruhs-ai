//! The live view's input rules: pointer, wheel and key events mapped to
//! the owner's [`ComputerInput`], as plain functions tested natively.

use bruhs_models::local_tools::{
    owner_key_route, to_viewport, to_viewport_clamped, ComputerInput, KeyKind, KeyRoute,
    MouseButton, MouseKind, Viewport, MAX_SCROLL_PX, MODIFIER_ALT, MODIFIER_CTRL, MODIFIER_META,
    MODIFIER_SHIFT,
};

/// Two presses this close in time and place count as one double click.
const MULTI_CLICK_MS: f64 = 500.0;
const MULTI_CLICK_PX: f64 = 4.0;
/// CSS px per wheel line, as browsers scroll a line.
const WHEEL_LINE_PX: f64 = 16.0;

/// Where a key pressed in the live view goes.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum KeyAction {
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
pub(super) fn modifier_bits(alt: bool, ctrl: bool, meta: bool, shift: bool) -> u8 {
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
pub(super) fn pressed_button(button: i16) -> MouseButton {
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

/// Owner pointer input at (`x`, `y`) on a live view element of size
/// `shown`. `buttons` and `button` are the DOM event's fields. A press
/// counts one click; a move carries the button held for a drag. Points on
/// the letterbox bands map to nothing, except a release or a drag, which
/// clamp to the frame's edge so the page always sees the button come up.
#[expect(
    clippy::too_many_arguments,
    reason = "the fields of one pointer event, kept plain so the mapping is testable natively"
)]
pub(super) fn pointer_input(
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
    let (x, y) = if clamps {
        to_viewport_clamped(x, y, shown, viewport)?
    } else {
        to_viewport(x, y, shown, viewport)?
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
pub(super) struct Press {
    pub at_ms: f64,
    pub x: f64,
    pub y: f64,
    pub button: MouseButton,
    pub clicks: u8,
}

/// How many clicks a press at `at_ms` makes after `last`: one more when it
/// comes soon after, nearby and with the same button, up to three.
pub(super) fn click_count(
    last: Option<Press>,
    at_ms: f64,
    x: f64,
    y: f64,
    button: MouseButton,
) -> u8 {
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
pub(super) fn with_clicks(input: ComputerInput, count: u8) -> ComputerInput {
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
pub(super) fn wheel_px(delta: f64, mode: u32, viewport: Viewport) -> f64 {
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
pub(super) fn wheel_input(
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
pub(super) fn key_action(key: &str, code: &str, modifiers: u8, mac: bool, down: bool) -> KeyAction {
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

/// The keys whose press went to the page, so each gets its release
/// whatever modifiers are held by then, and none is left down when the
/// view loses focus. Keys are told apart by their physical `code`.
#[derive(Debug, Default)]
pub(super) struct HeldKeys(Vec<(String, String)>);

impl HeldKeys {
    /// Where a key event goes, as [`key_action`] routes it, except that a
    /// release whose press reached the page always goes there too, and a
    /// release whose press didn't never does.
    pub(super) fn route(
        &mut self,
        key: &str,
        code: &str,
        modifiers: u8,
        mac: bool,
        down: bool,
    ) -> KeyAction {
        let id = if code.is_empty() { key } else { code };
        let action = key_action(key, code, modifiers, mac, down);
        if down {
            if matches!(action, KeyAction::Send(_)) && !self.0.iter().any(|(held, _)| held == id) {
                self.0.push((id.to_string(), key.to_string()));
            }
            return action;
        }
        match self.0.iter().position(|(held, _)| held == id) {
            Some(index) => {
                self.0.remove(index);
                KeyAction::Send(ComputerInput::Key {
                    kind: KeyKind::Up,
                    key: key.into(),
                    code: code.into(),
                    modifiers,
                })
            }
            None if action == KeyAction::Leave => KeyAction::Leave,
            None => KeyAction::App,
        }
    }

    /// Releases for every key still held, which are no longer.
    pub(super) fn release_all(&mut self) -> Vec<ComputerInput> {
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

    #[test]
    fn a_key_released_with_the_app_modifier_held_still_reaches_the_page() {
        let mut held = HeldKeys::default();
        // Shift goes down to the page, then ⌘ (the app's), then Shift comes
        // up while ⌘ is held: its release still goes to the page.
        assert!(matches!(
            held.route("Shift", "ShiftLeft", MODIFIER_SHIFT, true, true),
            KeyAction::Send(_)
        ));
        assert_eq!(
            held.route(
                "Meta",
                "MetaLeft",
                MODIFIER_SHIFT | MODIFIER_META,
                true,
                true
            ),
            KeyAction::App
        );
        assert_eq!(
            held.route("Shift", "ShiftLeft", MODIFIER_META, true, false),
            KeyAction::Send(ComputerInput::Key {
                kind: KeyKind::Up,
                key: "Shift".into(),
                code: "ShiftLeft".into(),
                modifiers: MODIFIER_META
            })
        );
        // A release whose press never reached the page stays with the app.
        assert_eq!(
            held.route("Meta", "MetaLeft", 0, true, false),
            KeyAction::App
        );
        assert_eq!(held.route("a", "KeyA", 0, true, false), KeyAction::App);
        assert_eq!(held.route("F6", "F6", 0, true, false), KeyAction::Leave);
    }

    #[test]
    fn keys_still_held_when_the_view_loses_focus_are_released_once() {
        let mut held = HeldKeys::default();
        assert!(matches!(
            held.route("a", "KeyA", 0, true, true),
            KeyAction::Send(_)
        ));
        // A repeat doesn't hold the key twice.
        assert!(matches!(
            held.route("a", "KeyA", 0, true, true),
            KeyAction::Send(_)
        ));
        assert!(matches!(
            held.route("ArrowDown", "ArrowDown", 0, true, true),
            KeyAction::Send(_)
        ));
        assert_eq!(
            held.release_all(),
            vec![
                ComputerInput::Key {
                    kind: KeyKind::Up,
                    key: "a".into(),
                    code: "KeyA".into(),
                    modifiers: 0
                },
                ComputerInput::Key {
                    kind: KeyKind::Up,
                    key: "ArrowDown".into(),
                    code: "ArrowDown".into(),
                    modifiers: 0
                },
            ]
        );
        assert!(held.release_all().is_empty());
        assert_eq!(held.route("a", "KeyA", 0, true, false), KeyAction::App);
    }
}
