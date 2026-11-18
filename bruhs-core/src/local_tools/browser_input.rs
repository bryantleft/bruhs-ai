//! CDP `Input.*` events for agent actions and the owner's live view input.
//!
//! Each function returns the params of one CDP call. Coordinates are CSS px
//! in the page view.

use bruhs_models::local_tools::*;
use serde_json::{json, Value};

pub(super) const MOUSE: &str = "Input.dispatchMouseEvent";
pub(super) const KEY: &str = "Input.dispatchKeyEvent";
pub(super) const TEXT: &str = "Input.insertText";

/// A left click: a move to the point, then a press and a release there.
pub(super) fn click_events(x: f64, y: f64) -> [Value; 3] {
    [
        mouse("mouseMoved", x, y, MouseButton::None, 0, 0, 0),
        mouse("mousePressed", x, y, MouseButton::Left, 1, 1, 0),
        mouse("mouseReleased", x, y, MouseButton::Left, 0, 1, 0),
    ]
}

/// A key down and its key up. A key with text types it; one without is a
/// raw key down, as Chrome's own input does for named keys.
pub(super) fn key_events(key: &CdpKey, modifiers: u8) -> [Value; 2] {
    let mut down = json!({
        "type": if key.text.is_some() { "keyDown" } else { "rawKeyDown" },
        "key": key.key,
        "code": key.code,
        "windowsVirtualKeyCode": key.key_code,
        "modifiers": modifiers,
    });
    if let Some(text) = &key.text {
        down["text"] = text.as_str().into();
        down["unmodifiedText"] = text.as_str().into();
    }
    let up = json!({
        "type": "keyUp",
        "key": key.key,
        "code": key.code,
        "windowsVirtualKeyCode": key.key_code,
        "modifiers": modifiers,
    });
    [down, up]
}

/// A wheel turn at a point, by `dx` and `dy` CSS px.
pub(super) fn wheel_event(x: f64, y: f64, dx: f64, dy: f64) -> Value {
    json!({"type": "mouseWheel", "x": x, "y": y, "deltaX": dx, "deltaY": dy})
}

/// The CDP calls for one owner input: mouse, wheel, key and text. Navigate,
/// Back and Reload are the caller's, so they give none.
pub(super) fn owner_events(input: &ComputerInput) -> Vec<(&'static str, Value)> {
    match input {
        ComputerInput::Mouse {
            kind,
            x,
            y,
            button,
            clicks,
            modifiers,
        } => {
            // Chrome reads a move without `buttons` as no button held, which
            // breaks drags; a release leaves no button held.
            let (kind, held, clicks) = match kind {
                MouseKind::Move => ("mouseMoved", held(*button), 0),
                MouseKind::Down => ("mousePressed", held(*button), (*clicks).max(1)),
                MouseKind::Up => ("mouseReleased", 0, (*clicks).max(1)),
            };
            vec![(
                MOUSE,
                mouse(kind, *x, *y, *button, held, clicks, *modifiers),
            )]
        }
        ComputerInput::Wheel { x, y, dx, dy } => vec![(MOUSE, wheel_event(*x, *y, *dx, *dy))],
        ComputerInput::Key {
            kind,
            key,
            code,
            modifiers,
        } => {
            let [down, up] = key_events(&owner_cdp_key(key, code), *modifiers);
            vec![(
                KEY,
                match kind {
                    KeyKind::Down => down,
                    KeyKind::Up => up,
                },
            )]
        }
        ComputerInput::Text { text } => vec![(TEXT, json!({"text": text}))],
        ComputerInput::Navigate { .. } | ComputerInput::Back | ComputerInput::Reload => Vec::new(),
    }
}

/// CDP's `buttons` bit for a held button: left 1, right 2, middle 4.
fn held(button: MouseButton) -> u8 {
    match button {
        MouseButton::None => 0,
        MouseButton::Left => 1,
        MouseButton::Right => 2,
        MouseButton::Middle => 4,
    }
}

fn mouse(
    kind: &str,
    x: f64,
    y: f64,
    button: MouseButton,
    buttons: u8,
    clicks: u8,
    modifiers: u8,
) -> Value {
    let button = match button {
        MouseButton::None => "none",
        MouseButton::Left => "left",
        MouseButton::Middle => "middle",
        MouseButton::Right => "right",
    };
    json!({
        "type": kind,
        "x": x,
        "y": y,
        "button": button,
        "buttons": buttons,
        "clickCount": clicks,
        "modifiers": modifiers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_is_a_move_a_press_and_a_release_with_the_left_button() {
        let [moved, pressed, released] = click_events(100.0, 50.5);
        assert_eq!(moved["type"], "mouseMoved");
        assert_eq!(
            (
                pressed["type"].as_str(),
                pressed["button"].as_str(),
                pressed["clickCount"].as_u64()
            ),
            (Some("mousePressed"), Some("left"), Some(1))
        );
        assert_eq!(
            (released["x"].as_f64(), released["y"].as_f64()),
            (Some(100.0), Some(50.5))
        );
    }

    #[test]
    fn keys_press_and_release_with_their_text_and_codes() {
        let [down, up] = key_events(&cdp_key("Enter").unwrap(), MODIFIER_SHIFT);
        assert_eq!(
            (
                down["type"].as_str(),
                down["text"].as_str(),
                down["windowsVirtualKeyCode"].as_u64(),
                down["modifiers"].as_u64()
            ),
            (Some("keyDown"), Some("\r"), Some(13), Some(8))
        );
        assert_eq!(up["type"], "keyUp");
        assert!(up.get("text").is_none());
    }

    #[test]
    fn owner_text_inserts_and_owner_keys_follow_their_kind() {
        assert_eq!(
            owner_events(&ComputerInput::Text { text: "pw".into() }),
            vec![("Input.insertText", serde_json::json!({"text": "pw"}))]
        );
        let up = owner_events(&ComputerInput::Key {
            kind: KeyKind::Up,
            key: "a".into(),
            code: "KeyA".into(),
            modifiers: 0,
        });
        assert_eq!(up.len(), 1);
        assert_eq!(up[0].1["type"], "keyUp");
        assert!(owner_events(&ComputerInput::Back).is_empty());
    }

    #[test]
    fn owner_drags_carry_the_held_button_on_every_move() {
        let mouse = |kind, button| {
            owner_events(&ComputerInput::Mouse {
                kind,
                x: 10.0,
                y: 20.0,
                button,
                clicks: 0,
                modifiers: MODIFIER_ALT,
            })
        };
        let [(method, moved)] = &mouse(MouseKind::Move, MouseButton::Left)[..] else {
            panic!("one move");
        };
        assert_eq!(*method, "Input.dispatchMouseEvent");
        assert_eq!(
            (
                moved["type"].as_str(),
                moved["button"].as_str(),
                moved["buttons"].as_u64(),
                moved["modifiers"].as_u64()
            ),
            (Some("mouseMoved"), Some("left"), Some(1), Some(1))
        );
        let hover = &mouse(MouseKind::Move, MouseButton::None)[0].1;
        assert_eq!(
            (hover["button"].as_str(), hover["buttons"].as_u64()),
            (Some("none"), Some(0))
        );
        let pressed = &mouse(MouseKind::Down, MouseButton::Right)[0].1;
        assert_eq!(
            (
                pressed["type"].as_str(),
                pressed["buttons"].as_u64(),
                pressed["clickCount"].as_u64()
            ),
            (Some("mousePressed"), Some(2), Some(1))
        );
        assert_eq!(
            mouse(MouseKind::Down, MouseButton::Middle)[0].1["buttons"],
            4
        );
        let released = &mouse(MouseKind::Up, MouseButton::Left)[0].1;
        assert_eq!(
            (
                released["type"].as_str(),
                released["button"].as_str(),
                released["buttons"].as_u64()
            ),
            (Some("mouseReleased"), Some("left"), Some(0))
        );
    }

    #[test]
    fn owner_wheels_and_unmapped_keys_reach_the_page() {
        let wheel = owner_events(&ComputerInput::Wheel {
            x: 5.0,
            y: 6.0,
            dx: -1.0,
            dy: 2.0,
        });
        assert_eq!(
            wheel,
            vec![("Input.dispatchMouseEvent", wheel_event(5.0, 6.0, -1.0, 2.0))]
        );
        let down = owner_events(&ComputerInput::Key {
            kind: KeyKind::Down,
            key: "F1".into(),
            code: "F1".into(),
            modifiers: MODIFIER_SHIFT,
        });
        assert_eq!(down.len(), 1);
        let (method, down) = &down[0];
        assert_eq!(*method, "Input.dispatchKeyEvent");
        assert_eq!(
            (
                down["type"].as_str(),
                down["key"].as_str(),
                down["windowsVirtualKeyCode"].as_u64(),
                down["modifiers"].as_u64()
            ),
            (Some("rawKeyDown"), Some("F1"), Some(112), Some(8))
        );
        assert!(down.get("text").is_none());
        assert!(owner_events(&ComputerInput::Reload).is_empty());
        assert!(owner_events(&ComputerInput::Navigate {
            url: "https://a.test/".into()
        })
        .is_empty());
    }
}
