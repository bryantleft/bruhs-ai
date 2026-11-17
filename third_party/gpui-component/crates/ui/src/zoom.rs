//! Page zoom, the way a browser zooms a page: every design length written
//! with [`px`] is in CSS pixels and is drawn at the current zoom, while
//! lengths measured from the window (bounds, mouse positions, scroll
//! offsets, text layout) are already in window points and go through
//! [`window_px`]. The theme's lengths (font size, radii) are rescaled with
//! the zoom, and the theme's font size sets the window's rem size (see
//! `Root`), so rem-based styles follow too.
//!
//! A local adaptation for bruhs-desktop's ⌘+ / ⌘− / ⌘0: upstream gpui has no
//! window zoom, and this keeps to its public API. See `VENDORED.md`.

use crate::Theme;
use gpui::{App, Pixels};
use std::cell::Cell;

thread_local! {
    // gpui lays out and paints on the thread that owns the app, so the zoom
    // lives there; each test thread starts at 100%.
    static ZOOM: Cell<f32> = const { Cell::new(1.0) };
}

/// The current zoom, 1.0 at 100%.
pub fn zoom() -> f32 {
    ZOOM.with(Cell::get)
}

/// A design length in CSS pixels, in window points at the current zoom.
#[inline]
pub fn px(css: f32) -> Pixels {
    gpui::px(css * zoom())
}

/// A length measured in window points, which the zoom must not scale again.
#[inline]
pub fn window_px(points: f32) -> Pixels {
    gpui::px(points)
}

/// A window measurement in CSS pixels, to compare with design lengths kept as
/// numbers (breakpoints, saved panel widths).
#[inline]
pub fn css_px(points: Pixels) -> f32 {
    f32::from(points) / zoom()
}

/// Sets the zoom and rescales the theme's lengths by the change. Re-render
/// the windows afterwards; cached measurements taken at the old zoom are the
/// caller's to reset.
pub fn set_zoom(zoom: f32, cx: &mut App) {
    let old = self::zoom();
    if !(zoom.is_finite() && zoom > 0.0) || (zoom - old).abs() < f32::EPSILON {
        return;
    }
    ZOOM.with(|cell| cell.set(zoom));
    if cx.has_global::<Theme>() {
        let scale = zoom / old;
        let theme = Theme::global_mut(cx);
        for length in [
            &mut theme.font_size,
            &mut theme.mono_font_size,
            &mut theme.radius,
            &mut theme.radius_lg,
            &mut theme.tile_grid_size,
            &mut theme.tile_radius,
        ] {
            *length = *length * scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn design_lengths_scale_and_measurements_do_not() {
        assert_eq!(px(10.0), gpui::px(10.0));
        ZOOM.with(|cell| cell.set(2.0));
        assert_eq!(px(10.0), gpui::px(20.0));
        assert_eq!(window_px(10.0), gpui::px(10.0));
        assert_eq!(css_px(gpui::px(1280.0)), 640.0);
        ZOOM.with(|cell| cell.set(1.0));
    }
}
