//! The live view's pointer rules: pointer events mapped to the owner's
//! [`ComputerInput`], as plain functions tested natively. Keys, held keys
//! and wheel summing are shared with the desktop in
//! `bruhs_models::local_tools`.

use bruhs_models::local_tools::{
    to_viewport, to_viewport_clamped, ComputerInput, MouseButton, MouseKind, Viewport,
    MAX_SCROLL_PX, WHEEL_LINE_PX,
};

/// Two presses this close in time and place count as one double click.
const MULTI_CLICK_MS: f64 = 500.0;
const MULTI_CLICK_PX: f64 = 4.0;

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

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::local_tools::{COMPUTER_VIEWPORT, MODIFIER_SHIFT};

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
    fn wheel_turns_become_page_pixels_within_the_budget() {
        let v = COMPUTER_VIEWPORT;
        assert_eq!(wheel_px(3.0, 1, v), 48.0);
        assert_eq!(wheel_px(1.0, 2, v), 800.0);
        assert_eq!(wheel_px(1e9, 0, v), MAX_SCROLL_PX);
        assert_eq!(wheel_px(f64::NAN, 0, v), 0.0);
    }
}
