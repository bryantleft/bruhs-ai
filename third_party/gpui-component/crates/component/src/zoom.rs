//! Page zoom: [`gpui_base::zoom`] plus [`set_zoom`], which also rescales the
//! styled theme's lengths. The theme's font size sets the window's rem size
//! (see `Root`), so rem-based styles follow. A local adaptation; see
//! `VENDORED.md`.

pub use gpui_base::zoom::{css_px, px, window_px, zoom};

use crate::Theme;
use gpui::App;

/// Sets the zoom and rescales the theme's lengths by the change. Re-render
/// the windows afterwards; cached measurements taken at the old zoom are the
/// caller's to reset.
pub fn set_zoom(zoom: f32, cx: &mut App) {
    let old = self::zoom();
    if !(zoom.is_finite() && zoom > 0.0) || (zoom - old).abs() < f32::EPSILON {
        return;
    }
    gpui_base::zoom::set_zoom_factor(zoom);
    if cx.has_global::<Theme>() {
        let scale = zoom / old;
        let theme = Theme::global_mut(cx);
        for length in [
            &mut theme.font_size,
            &mut theme.mono_font_size,
            &mut theme.radius,
            &mut theme.radius_lg,
        ] {
            *length = *length * scale;
        }
        // The scrollbar thumb's radius and the text view defaults follow.
        Theme::sync_base(cx);
    }
}
