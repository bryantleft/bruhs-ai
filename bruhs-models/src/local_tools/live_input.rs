//! The live view's input and reconnect rules both apps share: where an
//! owner key goes, which keys are held, how wheel turns sum within a frame
//! and how long a closed live view waits before it connects again.
use super::{
    owner_key_route, to_viewport, ComputerInput, KeyKind, KeyRoute, Viewport, MAX_SCROLL_PX,
};

/// CSS px per wheel line, as browsers scroll a line.
pub const WHEEL_LINE_PX: f64 = 16.0;
/// The first wait before a closed live view reconnects, doubled after each
/// failure up to [`BACKOFF_MAX_MS`].
pub const BACKOFF_START_MS: u64 = 1_000;
pub const BACKOFF_MAX_MS: u64 = 10_000;

/// Where a key pressed in the live view goes.
#[derive(Debug, Clone, PartialEq)]
pub enum KeyAction {
    /// To the page, as this input.
    Send(ComputerInput),
    /// Paste the clipboard's text into the page.
    Paste,
    /// Move focus to the toolbar.
    Leave,
    /// Leave it to the app's shortcuts.
    App,
}

/// Where a key in the live view goes, through [`owner_key_route`]. The
/// app's own modifier key (⌘ on a Mac, Ctrl elsewhere) stays with the app
/// too: its press carries its own bit, and its release must not reach the
/// page alone. A paste is taken on key down only. `key` and `code` are the
/// DOM names.
#[must_use]
pub fn key_action(key: &str, code: &str, modifiers: u8, mac: bool, down: bool) -> KeyAction {
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
pub struct HeldKeys(Vec<(String, String)>);

impl HeldKeys {
    /// Where a key event goes, as [`key_action`] routes it, except that a
    /// release whose press reached the page always goes there too (as the
    /// release event names it), and a release whose press didn't never
    /// does.
    pub fn route(
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
    pub fn release_all(&mut self) -> Vec<ComputerInput> {
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

/// A wheel turn at (`x`, `y`) on a live view element of size `shown`, in
/// page CSS px (`dx`, `dy`, the DOM's sign), added to any turn waiting to
/// be sent this frame. Turns over the letterbox map to nothing.
#[must_use]
pub fn wheel_input(
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
    let sum = |before: f64, turn: f64| {
        let turn = if turn.is_finite() { turn } else { 0.0 };
        (before + turn).clamp(-MAX_SCROLL_PX, MAX_SCROLL_PX)
    };
    Some(ComputerInput::Wheel {
        x,
        y,
        dx: sum(before_x, dx),
        dy: sum(before_y, dy),
    })
}

/// How long a closed live view waits before it connects again. Only a
/// frame proves the view works: the service sends its state first even
/// when the stream then fails, so a state message resets nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
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
    pub fn frame(&mut self) -> bool {
        let first = !self.framed;
        self.framed = true;
        self.wait_ms = BACKOFF_START_MS;
        first
    }

    /// The socket closed: the wait before the next one, doubled for the
    /// time after.
    pub fn closed(&mut self) -> u64 {
        let wait = self.wait_ms;
        self.wait_ms = wait.saturating_mul(2).min(BACKOFF_MAX_MS);
        self.framed = false;
        wait
    }

    /// A new socket opens: its first frame counts as first again.
    pub fn new_socket(&mut self) {
        self.framed = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_tools::{
        COMPUTER_VIEWPORT, MODIFIER_ALT, MODIFIER_CTRL, MODIFIER_META, MODIFIER_SHIFT,
    };

    fn key(kind: KeyKind, key: &str, code: &str, modifiers: u8) -> ComputerInput {
        ComputerInput::Key {
            kind,
            key: key.into(),
            code: code.into(),
            modifiers,
        }
    }

    #[test]
    fn keys_go_to_the_page_except_app_shortcuts_paste_and_f6() {
        assert_eq!(
            key_action("a", "KeyA", 0, true, true),
            KeyAction::Send(key(KeyKind::Down, "a", "KeyA", 0))
        );
        assert_eq!(
            key_action(".", "Period", MODIFIER_META, true, true),
            KeyAction::App
        );
        assert_eq!(
            key_action("v", "KeyV", MODIFIER_META, true, true),
            KeyAction::Paste
        );
        assert_eq!(key_action("F6", "F6", 0, true, true), KeyAction::Leave);
        assert!(matches!(
            key_action("Tab", "Tab", 0, true, true),
            KeyAction::Send(_)
        ));
        // Alt and Shift go to the page with the key.
        assert_eq!(
            key_action("a", "KeyA", MODIFIER_SHIFT | MODIFIER_ALT, false, false),
            KeyAction::Send(key(KeyKind::Up, "a", "KeyA", MODIFIER_SHIFT | MODIFIER_ALT))
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
        // Ctrl is the page's on a Mac, and ⌘ elsewhere.
        assert!(matches!(
            key_action("Control", "ControlLeft", MODIFIER_CTRL, true, true),
            KeyAction::Send(_)
        ));
        assert!(matches!(
            key_action("a", "KeyA", MODIFIER_CTRL, true, true),
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
            key_action(".", "Period", MODIFIER_CTRL, false, true),
            KeyAction::App
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
            KeyAction::Send(key(KeyKind::Up, "Shift", "ShiftLeft", MODIFIER_META))
        );
        // A release whose press never reached the page stays with the app.
        assert_eq!(
            held.route("Meta", "MetaLeft", 0, true, false),
            KeyAction::App
        );
        assert_eq!(held.route("a", "KeyA", 0, true, false), KeyAction::App);
        assert_eq!(held.route("F6", "F6", 0, true, false), KeyAction::Leave);
    }

    /// A release names the key as the release event does: `A` pressed with
    /// Shift comes up as `a` once Shift is up first.
    #[test]
    fn a_release_sends_the_release_events_key() {
        let mut held = HeldKeys::default();
        assert_eq!(
            held.route("A", "KeyA", MODIFIER_SHIFT, true, true),
            KeyAction::Send(key(KeyKind::Down, "A", "KeyA", MODIFIER_SHIFT))
        );
        assert_eq!(
            held.route("a", "KeyA", 0, true, false),
            KeyAction::Send(key(KeyKind::Up, "a", "KeyA", 0))
        );
        assert!(held.release_all().is_empty());
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
                key(KeyKind::Up, "a", "KeyA", 0),
                key(KeyKind::Up, "ArrowDown", "ArrowDown", 0),
            ]
        );
        assert!(held.release_all().is_empty());
        assert_eq!(held.route("a", "KeyA", 0, true, false), KeyAction::App);
        // A key with no code is held by its key.
        assert!(matches!(
            held.route("Dead", "", 0, true, true),
            KeyAction::Send(_)
        ));
        assert_eq!(
            held.release_all(),
            vec![key(KeyKind::Up, "Dead", "Dead", 0)]
        );
    }

    #[test]
    fn wheel_turns_sum_in_page_pixels_within_the_budget() {
        let v = COMPUTER_VIEWPORT;
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
        let odd = wheel_input(None, 320.0, 240.0, (640.0, 480.0), (f64::NAN, -1e12), v);
        assert_eq!(
            odd,
            Some(ComputerInput::Wheel {
                x: 640.0,
                y: 400.0,
                dx: 0.0,
                dy: -MAX_SCROLL_PX
            })
        );
        // The letterbox band.
        assert_eq!(
            wheel_input(None, 10.0, 5.0, (640.0, 480.0), (0.0, 1.0), v),
            None
        );
    }

    #[test]
    fn the_backoff_grows_until_a_frame_arrives() {
        let mut backoff = Backoff::default();
        // A stream that sends its state and fails: no frame, so each close
        // waits longer, up to the cap.
        let waits: Vec<u64> = (0..6).map(|_| backoff.closed()).collect();
        assert_eq!(waits, [1_000, 2_000, 4_000, 8_000, 10_000, 10_000]);
        // The first frame resets it; later frames on the socket aren't first.
        assert!(backoff.frame());
        assert!(!backoff.frame());
        assert_eq!(backoff.closed(), 1_000);
        // A new socket's first frame is first again.
        assert!(backoff.frame());
        backoff.new_socket();
        assert!(backoff.frame());
    }
}
