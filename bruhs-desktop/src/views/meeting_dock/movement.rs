use gpui::{Keystroke, Window};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Movement {
    By { x: f64, y: f64 },
    Center,
}

impl Movement {
    pub(super) fn from_key(key: &Keystroke) -> Option<Self> {
        if key.modifiers.control
            || key.modifiers.alt
            || key.modifiers.platform
            || key.modifiers.function
        {
            return None;
        }
        let step = if key.modifiers.shift { 40.0 } else { 10.0 };
        // AppKit's screen coordinates grow upward, unlike GPUI's content coordinates.
        match key.key.as_str() {
            "left" => Some(Self::By { x: -step, y: 0.0 }),
            "right" => Some(Self::By { x: step, y: 0.0 }),
            "up" => Some(Self::By { x: 0.0, y: step }),
            "down" => Some(Self::By { x: 0.0, y: -step }),
            "home" => Some(Self::Center),
            _ => None,
        }
    }
}

#[cfg(all(target_os = "macos", not(test)))]
pub(super) fn move_window(window: &Window, movement: Movement) -> Result<(), String> {
    let panel = super::native_panel(window)?;
    let screen = panel
        .screen()
        .ok_or("Meeting dock display is unavailable")?;
    let visible = screen.visibleFrame();
    let frame = panel.frame();
    let origin = match movement {
        Movement::By { x, y } => objc2_foundation::NSPoint::new(
            bounded_axis(
                frame.origin.x + x,
                visible.origin.x,
                visible.size.width,
                frame.size.width,
            ),
            bounded_axis(
                frame.origin.y + y,
                visible.origin.y,
                visible.size.height,
                frame.size.height,
            ),
        ),
        Movement::Center => objc2_foundation::NSPoint::new(
            visible.origin.x + (visible.size.width - frame.size.width).max(0.0) / 2.0,
            visible.origin.y + (visible.size.height - frame.size.height).max(0.0) / 2.0,
        ),
    };
    // Only our own panel moves, immediately and without activating another app.
    panel.setFrameOrigin(origin);
    Ok(())
}

#[cfg(any(not(target_os = "macos"), test))]
pub(super) fn move_window(_window: &Window, _movement: Movement) -> Result<(), String> {
    Err("Keyboard dock movement requires macOS".into())
}

#[cfg(any(target_os = "macos", test))]
fn bounded_axis(value: f64, min: f64, available: f64, extent: f64) -> f64 {
    value.clamp(min, min + (available - extent).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrow_keys_move_in_appkit_coordinates() {
        for (key, x, y) in [
            ("left", -10.0, 0.0),
            ("right", 10.0, 0.0),
            ("up", 0.0, 10.0),
            ("down", 0.0, -10.0),
        ] {
            assert_eq!(
                Movement::from_key(&Keystroke::parse(key).unwrap()),
                Some(Movement::By { x, y })
            );
        }
    }

    #[test]
    fn shift_moves_faster_and_home_centers() {
        assert_eq!(
            Movement::from_key(&Keystroke::parse("shift-right").unwrap()),
            Some(Movement::By { x: 40.0, y: 0.0 })
        );
        assert_eq!(
            Movement::from_key(&Keystroke::parse("home").unwrap()),
            Some(Movement::Center)
        );
    }

    #[test]
    fn unrelated_keys_and_system_shortcuts_do_not_move_the_dock() {
        for key in ["tab", "enter", "space", "cmd-left", "ctrl-right", "alt-up"] {
            assert_eq!(
                Movement::from_key(&Keystroke::parse(key).unwrap()),
                None,
                "{key}"
            );
        }
    }

    #[test]
    fn movement_stays_in_visible_screen_including_negative_origins() {
        assert_eq!(bounded_axis(-2000.0, -1920.0, 1920.0, 280.0), -1920.0);
        assert_eq!(bounded_axis(100.0, -1920.0, 1920.0, 280.0), -280.0);
        assert_eq!(bounded_axis(-1000.0, -1920.0, 1920.0, 280.0), -1000.0);
        assert_eq!(bounded_axis(900.0, 25.0, 800.0, 52.0), 773.0);
    }

    #[test]
    fn undersized_visible_area_pins_to_its_origin() {
        assert_eq!(bounded_axis(100.0, 25.0, 40.0, 52.0), 25.0);
    }
}
