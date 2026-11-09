//! Desktop twin of web `ui::braille::BrailleBruh`: the Bruh drawn in
//! braille-style dots (Bruhs AGENTS.md section 19). The dots come from
//! `bruhs_models::logic::dither`, so both apps place the same ones.

use crate::theme;
use bruhs_models::logic::dither::{bruh_dots, DotGrid, DotKind, BRUH_FACE_RGB, BRUH_MOUTH_RGB};
use gpui::{canvas, fill, point, px, rgb, size, Bounds, Hsla, IntoElement, Styled};

/// A static, dithered Bruh `size_px` square. Decorative, and it never moves.
pub fn braille_bruh(size_px: f32) -> impl IntoElement {
    let grid = DotGrid::for_size(size_px);
    let dots = bruh_dots(size_px, grid);
    let face: Hsla = rgb(BRUH_FACE_RGB).into();
    let mouth: Hsla = rgb(BRUH_MOUTH_RGB).into();
    let halo = theme::longan_700();
    canvas(
        |_bounds, _window, _cx| {},
        move |bounds, (), window, _cx| {
            let r = grid.dot;
            for dot in &dots {
                let color = match dot.kind {
                    DotKind::Face => face,
                    DotKind::Mouth => mouth,
                    DotKind::Halo => halo,
                };
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
