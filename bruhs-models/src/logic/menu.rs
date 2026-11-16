//! Floating menu rules shared by web and desktop: where a menu opens beside
//! the button that opened it, and how the arrow keys move through its items.

/// Gap between a menu and the button that opened it.
pub const MENU_GAP_PX: f64 = 4.0;
/// Room kept between a menu and the window edges.
pub const MENU_WINDOW_MARGIN_PX: f64 = 8.0;

/// A box in window coordinates: CSS pixels on web, GPUI pixels on desktop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuRect {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

/// Top-left corner of a `menu_width` × `menu_height` menu opened from
/// `trigger` in a `viewport_width` × `viewport_height` window: under the
/// trigger's left edge, above it when the window has no room below, and
/// clamped into the window margins.
pub fn menu_origin(
    trigger: MenuRect,
    (menu_width, menu_height): (f64, f64),
    (viewport_width, viewport_height): (f64, f64),
) -> (f64, f64) {
    let margin = MENU_WINDOW_MARGIN_PX;
    let max_top = (viewport_height - margin - menu_height).max(margin);
    let below = trigger.top + trigger.height + MENU_GAP_PX;
    let above = trigger.top - MENU_GAP_PX - menu_height;
    let top = if below <= max_top {
        below
    } else if above >= margin {
        above
    } else {
        max_top
    };
    let max_left = (viewport_width - margin - menu_width).max(margin);
    (trigger.left.max(margin).min(max_left), top)
}

/// Item to focus after `key` in a menu of `count` items, from the focused
/// item `current`: Down and Up wrap, Home and End jump. `None` for other keys
/// and for an empty menu. Takes web key names (`ArrowDown`) and GPUI key
/// names (`down`).
pub fn menu_step(current: Option<usize>, count: usize, key: &str) -> Option<usize> {
    let last = count.checked_sub(1)?;
    match key {
        "down" | "ArrowDown" => {
            Some(current.map_or(0, |index| if index >= last { 0 } else { index + 1 }))
        }
        "up" | "ArrowUp" => Some(current.map_or(last, |index| {
            if index == 0 || index > last {
                last
            } else {
                index - 1
            }
        })),
        "home" | "Home" => Some(0),
        "end" | "End" => Some(last),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MENU: (f64, f64) = (208.0, 290.0);

    fn trigger_at(left: f64, top: f64) -> MenuRect {
        MenuRect {
            left,
            top,
            width: 24.0,
            height: 24.0,
        }
    }

    #[test]
    fn a_menu_hangs_under_its_button_left_edge() {
        assert_eq!(
            menu_origin(trigger_at(178.0, 194.0), MENU, (1280.0, 800.0)),
            (178.0, 194.0 + 24.0 + MENU_GAP_PX)
        );
    }

    #[test]
    fn without_room_below_a_menu_opens_above_its_button() {
        assert_eq!(
            menu_origin(trigger_at(178.0, 700.0), MENU, (1280.0, 800.0)),
            (178.0, 700.0 - MENU_GAP_PX - MENU.1)
        );
    }

    #[test]
    fn a_short_window_keeps_the_menu_inside_its_margins() {
        // Neither side fits: pin the bottom edge to the bottom margin.
        let (_, top) = menu_origin(trigger_at(178.0, 200.0), MENU, (640.0, 320.0));
        assert_eq!(top, 320.0 - MENU_WINDOW_MARGIN_PX - MENU.1);
        let (_, tiny) = menu_origin(trigger_at(178.0, 40.0), MENU, (640.0, 200.0));
        assert_eq!(
            tiny, MENU_WINDOW_MARGIN_PX,
            "too tall: start at the top margin"
        );
    }

    #[test]
    fn a_narrow_window_pulls_the_menu_left_to_stay_inside() {
        let (left, _) = menu_origin(trigger_at(500.0, 100.0), MENU, (640.0, 800.0));
        assert_eq!(left, 640.0 - MENU_WINDOW_MARGIN_PX - MENU.0);
    }

    #[test]
    fn arrow_keys_wrap_and_home_and_end_jump_on_both_key_names() {
        for (down, up, home, end) in [
            ("down", "up", "home", "end"),
            ("ArrowDown", "ArrowUp", "Home", "End"),
        ] {
            assert_eq!(menu_step(None, 4, down), Some(0));
            assert_eq!(menu_step(Some(1), 4, down), Some(2));
            assert_eq!(menu_step(Some(3), 4, down), Some(0), "Down wraps");
            assert_eq!(menu_step(None, 4, up), Some(3));
            assert_eq!(menu_step(Some(0), 4, up), Some(3), "Up wraps");
            assert_eq!(menu_step(Some(2), 4, up), Some(1));
            assert_eq!(
                menu_step(Some(9), 4, up),
                Some(3),
                "a stale index lands on the last item"
            );
            assert_eq!(menu_step(Some(2), 4, home), Some(0));
            assert_eq!(menu_step(Some(0), 4, end), Some(3));
        }
        assert_eq!(menu_step(Some(0), 4, "Enter"), None);
        assert_eq!(
            menu_step(None, 0, "ArrowDown"),
            None,
            "an empty menu has nothing to focus"
        );
    }
}
