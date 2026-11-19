//! A bruh's Computer: its state, the owner's requests and input, the live
//! view protocol and the key and point mapping both apps share.
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};

use super::{
    in_view, validate_navigation, within_scroll, within_type_budget, SCROLL_TOO_FAR, TOO_MUCH_TEXT,
};

/// The page view a Computer's browser renders, in CSS px.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
}

/// The headless browser's view: 1280 by 800 CSS px.
pub const COMPUTER_VIEWPORT: Viewport = Viewport {
    width: 1280,
    height: 800,
};

impl Default for Viewport {
    fn default() -> Self {
        COMPUTER_VIEWPORT
    }
}

/// Who drives a Computer's browser right now.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Control {
    #[default]
    Bruh,
    Owner,
    /// A newer service's control this app doesn't know, so the rest of its
    /// state still reads.
    #[serde(other)]
    Other,
}

/// What both apps show about a bruh's Computer.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct ComputerState {
    pub agent_id: String,
    /// Present while the browser runs.
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub control: Control,
    /// The browser runs in a visible window on this computer.
    #[serde(default)]
    pub headed: bool,
    #[serde(default)]
    pub viewport: Viewport,
    #[serde(default)]
    pub error: Option<String>,
    /// This system can't run a bruh's browser (off macOS and Linux), so
    /// none ever opens here.
    #[serde(default)]
    pub unsupported: bool,
}

/// What the owner asks of a bruh's Computer.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ComputerRequest {
    Open,
    Close,
    TakeControl,
    HandBack,
    OpenWindow,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MouseKind {
    Move,
    Down,
    Up,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    #[default]
    None,
    Left,
    Middle,
    Right,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KeyKind {
    Down,
    Up,
}

/// The owner's input into the live view while they hold control. Points are
/// in the page view's CSS px; `modifiers` holds the `MODIFIER_*` bits.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ComputerInput {
    Mouse {
        kind: MouseKind,
        x: f64,
        y: f64,
        /// The pressed button; on a move, the button held for a drag.
        #[serde(default)]
        button: MouseButton,
        #[serde(default)]
        clicks: u8,
        #[serde(default)]
        modifiers: u8,
    },
    Wheel {
        x: f64,
        y: f64,
        dx: f64,
        dy: f64,
    },
    Key {
        kind: KeyKind,
        key: String,
        #[serde(default)]
        code: String,
        #[serde(default)]
        modifiers: u8,
    },
    /// Text inserted as typed, such as a paste.
    Text {
        text: String,
    },
    Navigate {
        url: String,
    },
    Back,
    Reload,
}

/// The longest DOM key or code name owner input carries, in bytes.
const MAX_KEY_BYTES: usize = 32;
const ALL_MODIFIERS: u8 = MODIFIER_ALT | MODIFIER_CTRL | MODIFIER_META | MODIFIER_SHIFT;

/// Check owner input against the view it acts on and its budgets. Owner
/// navigation follows the agent's URL rules.
pub fn validate_input(input: &ComputerInput, viewport: Viewport) -> Result<(), &'static str> {
    match input {
        ComputerInput::Mouse {
            x, y, modifiers, ..
        } => {
            if !in_view(*x, *y, viewport) {
                return Err("Point inside the page view.");
            }
            valid_modifiers(*modifiers)
        }
        ComputerInput::Wheel { x, y, dx, dy } => {
            if !in_view(*x, *y, viewport) {
                return Err("Scroll inside the page view.");
            }
            if !within_scroll(*dx, *dy) {
                return Err(SCROLL_TOO_FAR);
            }
            Ok(())
        }
        ComputerInput::Key {
            key,
            code,
            modifiers,
            ..
        } => {
            if key.is_empty() || key.len() > MAX_KEY_BYTES || code.len() > MAX_KEY_BYTES {
                return Err("Send one key name of at most 32 bytes.");
            }
            valid_modifiers(*modifiers)
        }
        ComputerInput::Text { text } if !within_type_budget(text) => Err(TOO_MUCH_TEXT),
        ComputerInput::Navigate { url } => validate_navigation(url),
        ComputerInput::Text { .. } | ComputerInput::Back | ComputerInput::Reload => Ok(()),
    }
}

fn valid_modifiers(modifiers: u8) -> Result<(), &'static str> {
    if modifiers & !ALL_MODIFIERS == 0 {
        Ok(())
    } else {
        Err("Use only the Alt, Ctrl, Meta and Shift modifier bits.")
    }
}

/// What the server sends a live view as text frames. Frames themselves are
/// binary JPEG messages.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ScreencastMessage {
    State(ComputerState),
    Error {
        error: String,
    },
    /// A newer service's message this app doesn't know; clients skip it.
    #[serde(other)]
    Other,
}

/// What a live view sends the server, besides [`FRAME_ACK`].
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ScreencastClientMessage {
    Input { input: ComputerInput },
}

/// The bare text frame a live view sends after decoding each frame.
pub const FRAME_ACK: &str = "frame_received";

/// Unreserved path characters stay as they are; everything else is encoded.
const PATH_SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// The live view's WebSocket path for a browser session.
#[must_use]
pub fn screencast_path(session_id: &str) -> String {
    format!(
        "/api/local-tools/browser/{}/screencast",
        utf8_percent_encode(session_id, PATH_SEGMENT)
    )
}

/// The live view's WebSocket address on the API at `base`: `http` becomes
/// `ws` and `https` becomes `wss`, and a trailing slash is dropped. Both
/// apps build the address here so they never differ.
#[must_use]
pub fn screencast_url(base: &str, session_id: &str) -> String {
    let base = base.trim_end_matches('/');
    let ws_base = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_string()
    };
    format!("{ws_base}{}", screencast_path(session_id))
}

/// CDP's modifier bits, as `Input.dispatchKeyEvent` and
/// `Input.dispatchMouseEvent` take them.
pub const MODIFIER_ALT: u8 = 1;
pub const MODIFIER_CTRL: u8 = 2;
pub const MODIFIER_META: u8 = 4;
pub const MODIFIER_SHIFT: u8 = 8;

/// The `MODIFIER_*` bits of the modifier keys held.
#[must_use]
pub fn modifier_bits(alt: bool, ctrl: bool, meta: bool, shift: bool) -> u8 {
    [
        (alt, MODIFIER_ALT),
        (ctrl, MODIFIER_CTRL),
        (meta, MODIFIER_META),
        (shift, MODIFIER_SHIFT),
    ]
    .into_iter()
    .filter(|(held, _)| *held)
    .fold(0, |bits, (_, bit)| bits | bit)
}

/// The fields CDP's `Input.dispatchKeyEvent` needs for one key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CdpKey {
    pub key: String,
    pub code: String,
    /// The Windows virtual key code.
    pub key_code: u32,
    /// The text a key down inserts, if any.
    pub text: Option<String>,
}

/// Map a DOM key name to CDP's key fields: the named keys in
/// [`super::AGENT_KEYS`] (and `" "` for Space), or one printable character.
#[must_use]
pub fn cdp_key(key: &str) -> Option<CdpKey> {
    let named = |key: &str, code: &str, key_code: u32, text: Option<&str>| CdpKey {
        key: key.into(),
        code: code.into(),
        key_code,
        text: text.map(Into::into),
    };
    let mapped = match key {
        "Enter" => named("Enter", "Enter", 13, Some("\r")),
        "Tab" => named("Tab", "Tab", 9, None),
        "Escape" => named("Escape", "Escape", 27, None),
        "Backspace" => named("Backspace", "Backspace", 8, None),
        "Delete" => named("Delete", "Delete", 46, None),
        "Space" | " " => named(" ", "Space", 32, Some(" ")),
        "ArrowLeft" => named("ArrowLeft", "ArrowLeft", 37, None),
        "ArrowUp" => named("ArrowUp", "ArrowUp", 38, None),
        "ArrowRight" => named("ArrowRight", "ArrowRight", 39, None),
        "ArrowDown" => named("ArrowDown", "ArrowDown", 40, None),
        "Home" => named("Home", "Home", 36, None),
        "End" => named("End", "End", 35, None),
        "PageUp" => named("PageUp", "PageUp", 33, None),
        "PageDown" => named("PageDown", "PageDown", 34, None),
        _ => return printable_key(key),
    };
    Some(mapped)
}

fn printable_key(key: &str) -> Option<CdpKey> {
    let mut chars = key.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return None;
    };
    if c.is_control() {
        return None;
    }
    let upper = c.to_ascii_uppercase();
    let (code, key_code) = if c.is_ascii_alphabetic() {
        (format!("Key{upper}"), u32::from(upper))
    } else if c.is_ascii_digit() {
        (format!("Digit{c}"), u32::from(c))
    } else {
        (String::new(), 0)
    };
    Some(CdpKey {
        key: key.into(),
        code,
        key_code,
        text: Some(key.into()),
    })
}

/// Map an owner key from the live view to CDP's key fields. Keys
/// [`cdp_key`] maps keep its fields, with the client's `code` where it has
/// none (punctuation). Any other key is forwarded as the client named it,
/// with a virtual key code for function keys, Insert and the modifiers (0
/// otherwise) and no text. Agent keys stay on [`super::AGENT_KEYS`].
#[must_use]
pub fn owner_cdp_key(key: &str, code: &str) -> CdpKey {
    if let Some(mut mapped) = cdp_key(key) {
        if mapped.code.is_empty() {
            mapped.code = code.into();
        }
        return mapped;
    }
    CdpKey {
        key: key.into(),
        code: code.into(),
        key_code: forwarded_key_code(key),
        text: None,
    }
}

/// Windows virtual key codes for owner keys [`cdp_key`] doesn't map.
fn forwarded_key_code(key: &str) -> u32 {
    if let Some(number) = key
        .strip_prefix('F')
        .and_then(|number| number.parse::<u32>().ok())
        .filter(|number| (1..=24).contains(number))
    {
        return 111 + number;
    }
    match key {
        "Insert" => 45,
        "Shift" => 16,
        "Control" => 17,
        "Alt" => 18,
        "Meta" => 91,
        "CapsLock" => 20,
        _ => 0,
    }
}

/// Where an owner key press in the live view goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyRoute {
    /// Forward to the page.
    Page,
    /// Paste the clipboard into the page as text.
    Paste,
    /// Move focus out of the live view.
    Leave,
    /// Leave it to the app's own shortcuts.
    App,
}

/// F6 leaves the live view. ⌘ on a Mac, or Ctrl elsewhere, keeps a key
/// with the app, except V, which pastes. Every other key goes to the page.
#[must_use]
pub fn owner_key_route(key: &str, modifiers: u8, mac: bool) -> KeyRoute {
    if key == "F6" {
        return KeyRoute::Leave;
    }
    let app_modifier = if mac { MODIFIER_META } else { MODIFIER_CTRL };
    if modifiers & app_modifier == 0 {
        KeyRoute::Page
    } else if key.eq_ignore_ascii_case("v") {
        KeyRoute::Paste
    } else {
        KeyRoute::App
    }
}

/// Map a point in the live view element of size `shown` to the page view,
/// with the frame drawn `object-fit: contain`. Points on the letterbox bands
/// map to nothing.
#[must_use]
pub fn to_viewport(x: f64, y: f64, shown: (f64, f64), viewport: Viewport) -> Option<(f64, f64)> {
    let point = frame_point(x, y, shown, viewport)?;
    in_view(point.0, point.1, viewport).then_some(point)
}

/// Like [`to_viewport`], but a point outside the frame clamps to its
/// nearest edge, so a drag released outside the live view still ends, with
/// its mouse up, inside the page. Nothing maps while the view has no size.
#[must_use]
pub fn to_viewport_clamped(
    x: f64,
    y: f64,
    shown: (f64, f64),
    viewport: Viewport,
) -> Option<(f64, f64)> {
    let (px, py) = frame_point(x, y, shown, viewport)?;
    Some((
        px.clamp(0.0, f64::from(viewport.width)),
        py.clamp(0.0, f64::from(viewport.height)),
    ))
}

/// The page view point under (`x`, `y`) on an element of size `shown` that
/// draws the frame `object-fit: contain`, unbounded.
fn frame_point(x: f64, y: f64, shown: (f64, f64), viewport: Viewport) -> Option<(f64, f64)> {
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
    Some(((x - left) / scale, (y - top) / scale))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_tools::{AGENT_KEYS, MAX_SCROLL_PX, MAX_TYPE_CHARS};

    #[test]
    fn screencast_urls_use_the_api_origin_with_ws() {
        assert_eq!(
            screencast_url("http://127.0.0.1:3001", "s1"),
            "ws://127.0.0.1:3001/api/local-tools/browser/s1/screencast"
        );
        assert_eq!(
            screencast_url("https://api.bruhs.localhost", "s1"),
            "wss://api.bruhs.localhost/api/local-tools/browser/s1/screencast"
        );
        assert_eq!(
            screencast_url("http://127.0.0.1:3001/", "a b"),
            "ws://127.0.0.1:3001/api/local-tools/browser/a%20b/screencast"
        );
    }

    #[test]
    fn every_agent_key_maps_to_cdp() {
        for key in AGENT_KEYS {
            let mapped = cdp_key(key).unwrap_or_else(|| panic!("{key}"));
            assert!(mapped.key_code > 0 && !mapped.code.is_empty(), "{key}");
        }
    }

    #[test]
    fn modifier_bits_fold_the_held_keys() {
        assert_eq!(modifier_bits(false, false, false, false), 0);
        assert_eq!(
            modifier_bits(true, true, true, true),
            MODIFIER_ALT | MODIFIER_CTRL | MODIFIER_META | MODIFIER_SHIFT
        );
        assert_eq!(
            modifier_bits(false, true, false, true),
            MODIFIER_CTRL | MODIFIER_SHIFT
        );
    }

    #[test]
    fn owner_keys_cdp_cannot_map_are_forwarded_without_text() {
        assert_eq!(
            owner_cdp_key("F1", "F1"),
            CdpKey {
                key: "F1".into(),
                code: "F1".into(),
                key_code: 112,
                text: None
            }
        );
        assert_eq!(owner_cdp_key("F12", "F12").key_code, 123);
        assert_eq!(owner_cdp_key("F13", "F13").key_code, 124);
        assert_eq!(owner_cdp_key("F24", "F24").key_code, 135);
        assert_eq!(owner_cdp_key("F25", "F25").key_code, 0);
        assert_eq!(owner_cdp_key("Insert", "Insert").key_code, 45);
        let shift = owner_cdp_key("Shift", "ShiftLeft");
        assert_eq!(
            (shift.code.as_str(), shift.key_code, shift.text),
            ("ShiftLeft", 16, None)
        );
        assert_eq!(owner_cdp_key("Meta", "MetaLeft").key_code, 91);
        assert_eq!(owner_cdp_key("Unidentified", "").key_code, 0);
        // Punctuation keeps its text and takes the client's physical code.
        let bang = owner_cdp_key("!", "Digit1");
        assert_eq!(
            (bang.code.as_str(), bang.text.as_deref()),
            ("Digit1", Some("!"))
        );
        // A key cdp_key maps keeps its own fields.
        assert_eq!(
            owner_cdp_key("Enter", "NumpadEnter"),
            cdp_key("Enter").unwrap()
        );
    }

    #[test]
    fn state_messages_carry_the_state_fields() {
        let state = ComputerState {
            agent_id: "a".into(),
            session_id: Some("s".into()),
            url: Some("https://a.test/".into()),
            control: Control::Owner,
            ..ComputerState::default()
        };
        let message = ScreencastMessage::State(state);
        let wire = serde_json::json!({
            "type": "state",
            "agent_id": "a",
            "session_id": "s",
            "url": "https://a.test/",
            "control": "owner",
            "headed": false,
            "viewport": {"width": 1280, "height": 800},
            "error": null,
            "unsupported": false,
        });
        assert_eq!(serde_json::to_value(&message).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<ScreencastMessage>(wire).unwrap(),
            message
        );
        // An older service sends no `unsupported`: it reads as supported.
        let older = serde_json::from_value::<ComputerState>(serde_json::json!({"agent_id": "a"}));
        assert!(!older.unwrap().unsupported);
    }

    /// A newer service's control or message never stops a state from
    /// reading: the rest of it still shows.
    #[test]
    fn unknown_controls_and_messages_still_read() {
        let state = serde_json::from_value::<ScreencastMessage>(serde_json::json!({
            "type": "state",
            "agent_id": "a",
            "session_id": "s",
            "control": "shared",
        }))
        .unwrap();
        let ScreencastMessage::State(state) = state else {
            panic!("not a state: {state:?}");
        };
        assert_eq!(state.control, Control::Other);
        assert_eq!(state.session_id.as_deref(), Some("s"));
        assert_eq!(
            serde_json::from_value::<ScreencastMessage>(
                serde_json::json!({"type": "cursor", "x": 1})
            )
            .unwrap(),
            ScreencastMessage::Other
        );
    }

    #[test]
    fn owner_wheels_keys_and_modifiers_are_bounded() {
        let v = COMPUTER_VIEWPORT;
        let wheel = |x: f64, dy: f64| ComputerInput::Wheel {
            x,
            y: 10.0,
            dx: 0.0,
            dy,
        };
        assert!(validate_input(&wheel(10.0, -MAX_SCROLL_PX), v).is_ok());
        assert_eq!(
            validate_input(&wheel(-1.0, 1.0), v),
            Err("Scroll inside the page view.")
        );
        for dy in [MAX_SCROLL_PX + 1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                validate_input(&wheel(10.0, dy), v),
                Err("Scroll at most 10,000 pixels at once.")
            );
        }
        let key = |key: &str, code: &str, modifiers: u8| ComputerInput::Key {
            kind: KeyKind::Down,
            key: key.into(),
            code: code.into(),
            modifiers,
        };
        let all = MODIFIER_ALT | MODIFIER_CTRL | MODIFIER_META | MODIFIER_SHIFT;
        assert!(validate_input(&key("a", "KeyA", all), v).is_ok());
        assert!(validate_input(&key("a", "KeyA", 16), v).is_err());
        assert!(validate_input(&key("", "KeyA", 0), v).is_err());
        assert!(validate_input(&key("a", &"x".repeat(32), 0), v).is_ok());
        assert!(validate_input(&key("a", &"x".repeat(33), 0), v).is_err());
        let mouse = |modifiers: u8| ComputerInput::Mouse {
            kind: MouseKind::Down,
            x: 1.0,
            y: 1.0,
            button: MouseButton::Left,
            clicks: 1,
            modifiers,
        };
        assert!(validate_input(&mouse(MODIFIER_SHIFT), v).is_ok());
        assert!(validate_input(&mouse(0x80), v).is_err());
    }

    #[test]
    fn computer_messages_have_stable_wire_names() {
        assert_eq!(
            serde_json::from_str::<ComputerRequest>(r#"{"action":"take_control"}"#).unwrap(),
            ComputerRequest::TakeControl
        );
        let down: ComputerInput = serde_json::from_str(
            r#"{"type":"mouse","kind":"down","x":5,"y":6,"button":"left","clicks":1}"#,
        )
        .unwrap();
        assert!(matches!(
            down,
            ComputerInput::Mouse {
                kind: MouseKind::Down,
                button: MouseButton::Left,
                clicks: 1,
                ..
            }
        ));
        assert_eq!(
            serde_json::to_value(ScreencastMessage::Error { error: "x".into() }).unwrap(),
            serde_json::json!({"type": "error", "error": "x"})
        );
        let state: ComputerState = serde_json::from_str(r#"{"agent_id":"a","future":1}"#).unwrap();
        assert_eq!(
            (state.control, state.headed, state.viewport),
            (Control::Bruh, false, COMPUTER_VIEWPORT)
        );
        assert_eq!(
            screencast_path("s 1"),
            "/api/local-tools/browser/s%201/screencast"
        );
        let back = ScreencastClientMessage::Input {
            input: ComputerInput::Back,
        };
        assert_eq!(
            serde_json::to_value(&back).unwrap(),
            serde_json::json!({"type": "input", "input": {"type": "back"}})
        );
        assert_eq!(
            serde_json::from_str::<ScreencastClientMessage>(
                r#"{"type":"input","input":{"type":"back"}}"#
            )
            .unwrap(),
            back
        );
    }

    #[test]
    fn keys_map_to_cdp_fields() {
        let enter = cdp_key("Enter").unwrap();
        assert_eq!(
            (enter.code.as_str(), enter.key_code, enter.text.as_deref()),
            ("Enter", 13, Some("\r"))
        );
        let a = cdp_key("a").unwrap();
        assert_eq!(
            (a.code.as_str(), a.key_code, a.text.as_deref()),
            ("KeyA", 65, Some("a"))
        );
        let seven = cdp_key("7").unwrap();
        assert_eq!((seven.code.as_str(), seven.key_code), ("Digit7", 55));
        assert_eq!(cdp_key("Space").unwrap().text.as_deref(), Some(" "));
        assert_eq!(cdp_key("é").unwrap().text.as_deref(), Some("é"));
        assert!(cdp_key("").is_none() && cdp_key("NotAKey").is_none());
    }

    #[test]
    fn owner_keys_route_app_shortcuts_paste_and_leave() {
        assert_eq!(owner_key_route("F6", 0, true), KeyRoute::Leave);
        assert_eq!(owner_key_route(".", MODIFIER_META, true), KeyRoute::App);
        assert_eq!(owner_key_route("k", MODIFIER_META, true), KeyRoute::App);
        assert_eq!(owner_key_route("v", MODIFIER_META, true), KeyRoute::Paste);
        assert_eq!(owner_key_route("V", MODIFIER_CTRL, false), KeyRoute::Paste);
        assert_eq!(owner_key_route("Tab", 0, true), KeyRoute::Page);
        assert_eq!(owner_key_route("a", MODIFIER_CTRL, true), KeyRoute::Page); // Ctrl on a Mac is the page's
        assert_eq!(owner_key_route("Escape", 0, false), KeyRoute::Page);
    }

    #[test]
    fn live_view_points_map_into_the_page_view() {
        let v = COMPUTER_VIEWPORT;
        assert_eq!(
            to_viewport(320.0, 200.0, (640.0, 400.0), v),
            Some((640.0, 400.0))
        );
        // 640x480 letterboxes the 16:10 frame with 40px bands above and below.
        assert_eq!(to_viewport(10.0, 20.0, (640.0, 480.0), v), None);
        assert_eq!(
            to_viewport(640.0, 440.0, (640.0, 480.0), v),
            Some((1280.0, 800.0))
        );
        assert_eq!(to_viewport(1.0, 1.0, (0.0, 0.0), v), None);
    }

    #[test]
    fn points_outside_the_frame_clamp_to_its_edges() {
        let v = COMPUTER_VIEWPORT;
        // Inside the frame both mappings agree.
        assert_eq!(
            to_viewport_clamped(320.0, 240.0, (640.0, 480.0), v),
            to_viewport(320.0, 240.0, (640.0, 480.0), v)
        );
        // Below and right of a 640x480 view: the frame's bottom right corner.
        assert_eq!(
            to_viewport_clamped(900.0, 470.0, (640.0, 480.0), v),
            Some((1280.0, 800.0))
        );
        // On the top band and left of the element: the top left corner.
        assert_eq!(
            to_viewport_clamped(-5.0, 10.0, (640.0, 480.0), v),
            Some((0.0, 0.0))
        );
        assert_eq!(to_viewport_clamped(1.0, 1.0, (0.0, 0.0), v), None);
        assert_eq!(to_viewport_clamped(f64::NAN, 1.0, (640.0, 480.0), v), None);
    }

    #[test]
    fn owner_input_is_bounded_and_never_reaches_this_computer() {
        let v = COMPUTER_VIEWPORT;
        assert!(validate_input(
            &ComputerInput::Mouse {
                kind: MouseKind::Move,
                x: 2000.0,
                y: 1.0,
                button: MouseButton::None,
                clicks: 0,
                modifiers: 0
            },
            v
        )
        .is_err());
        assert!(validate_input(
            &ComputerInput::Text {
                text: "x".repeat(MAX_TYPE_CHARS + 1)
            },
            v
        )
        .is_err());
        assert_eq!(
            validate_input(
                &ComputerInput::Navigate {
                    url: "http://127.1:3001/".into()
                },
                v
            ),
            Err(crate::crew::LOCAL_HOST_DENIAL)
        );
        assert!(validate_input(
            &ComputerInput::Navigate {
                url: "file:///etc/passwd".into()
            },
            v
        )
        .is_err());
        assert!(validate_input(
            &ComputerInput::Key {
                kind: KeyKind::Down,
                key: "x".repeat(40),
                code: String::new(),
                modifiers: 0
            },
            v
        )
        .is_err());
    }
}
