//! Page zoom for the desktop window, the counterpart of a browser's
//! ⌘+ / ⌘− / ⌘0 on the web app: the window draws its content larger in a
//! smaller layout viewport, so pages reflow at the web's widths
//! (`gpui::Window::set_zoom`). The steps are Chrome's, from 67% to 300%;
//! below 67% the window header gets shorter than the macOS traffic lights.

/// The zoom steps, smallest first.
pub const LEVELS: [f32; 12] = [
    0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0,
];
/// 100%.
pub const DEFAULT: f32 = 1.0;
const EPSILON: f32 = 0.001;

/// The next step up from `current`, or the largest.
pub fn zoom_in(current: f32) -> f32 {
    LEVELS
        .into_iter()
        .find(|level| *level > current + EPSILON)
        .unwrap_or(LEVELS[LEVELS.len() - 1])
}

/// The next step down from `current`, or the smallest.
pub fn zoom_out(current: f32) -> f32 {
    LEVELS
        .into_iter()
        .rev()
        .find(|level| *level < current - EPSILON)
        .unwrap_or(LEVELS[0])
}

/// A saved zoom, or 100% when it is missing or outside the steps' range.
pub fn restore(saved: Option<f32>) -> f32 {
    saved
        .filter(|zoom| {
            zoom.is_finite()
                && *zoom >= LEVELS[0] - EPSILON
                && *zoom <= LEVELS[LEVELS.len() - 1] + EPSILON
        })
        .unwrap_or(DEFAULT)
}

/// Whether `zoom` is 100%, so it need not be saved.
pub fn is_default(zoom: f32) -> bool {
    (zoom - DEFAULT).abs() < EPSILON
}

/// The macOS traffic lights stay in window points while the content zooms.
/// Space reserved for them, in content points, is their window-point width
/// divided by the zoom.
pub fn traffic_light_inset(inset_px: f32, zoom: f32) -> f32 {
    inset_px / zoom
}

/// The traffic lights' top, in window points, that keeps them centred in a
/// header `header_px` content points tall: 15pt at 100% for the 44pt header.
pub fn traffic_light_top(header_px: f32, zoom: f32) -> f32 {
    15.0 + header_px / 2.0 * (zoom - 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_follow_chrome_and_stop_at_the_ends() {
        assert_eq!(zoom_in(1.0), 1.1);
        assert_eq!(zoom_in(1.1), 1.25);
        assert_eq!(zoom_out(1.0), 0.9);
        assert_eq!(zoom_in(3.0), 3.0);
        assert_eq!(zoom_out(0.67), 0.67);
        // Off-step values move to the neighbouring step.
        assert_eq!(zoom_in(1.2), 1.25);
        assert_eq!(zoom_out(1.2), 1.1);
    }

    #[test]
    fn saved_zoom_is_restored_only_in_range() {
        assert_eq!(restore(Some(1.5)), 1.5);
        assert_eq!(restore(None), 1.0);
        assert_eq!(restore(Some(0.25)), 1.0);
        assert_eq!(restore(Some(f32::NAN)), 1.0);
        assert!(is_default(1.0) && !is_default(1.1));
    }

    #[test]
    fn traffic_lights_keep_their_window_place() {
        assert_eq!(traffic_light_inset(88.0, 2.0), 44.0);
        assert_eq!(traffic_light_top(44.0, 1.0), 15.0);
        assert_eq!(traffic_light_top(44.0, 2.0), 37.0);
    }
}
