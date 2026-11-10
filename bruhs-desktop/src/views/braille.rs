//! Desktop twin of web `ui::braille::BrailleBruh`: the Bruh drawn in
//! a halftone of dots (Bruhs AGENTS.md section 19). The dots come from
//! `bruhs_models::logic::dither`, so both apps place the same ones.

use bruhs_models::logic::dither::bruh_dots;
use gpui::{canvas, fill, point, px, rgb, size, Bounds, Hsla, IntoElement, Styled};

/// A static, dithered Bruh `size_px` square. Decorative, and it never moves.
pub fn braille_bruh(size_px: f32) -> impl IntoElement {
    let dots = bruh_dots(size_px);
    canvas(
        |_bounds, _window, _cx| {},
        move |bounds, (), window, _cx| {
            for dot in &dots {
                let r = dot.radius;
                let color: Hsla = rgb(dot.rgb()).into();
                let origin = point(
                    bounds.origin.x + px(dot.x - r),
                    bounds.origin.y + px(dot.y - r),
                );
                window.paint_quad(
                    fill(Bounds::new(origin, size(px(r * 2.0), px(r * 2.0))), color)
                        .corner_radii(px(r)),
                );
            }
        },
    )
    .size(px(size_px))
    .flex_none()
}
