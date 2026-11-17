//! Desktop twin of web `ui::braille::BrailleBruh`: the Bruh drawn in
//! a halftone of dots (Bruhs AGENTS.md section 19). The dots come from
//! `bruhs_models::logic::dither`, so both apps place the same ones.

use bruhs_models::crew::{FaceView, RING_RADIUS, RING_STROKE_PX};
use bruhs_models::logic::dither::bruh_dots_styled;
use gpui::{
    canvas, fill, point, px, quad, rgb, size, transparent_black, BorderStyle, Bounds, Hsla,
    IntoElement, Styled,
};

/// A static, dithered Bruh `size_px` square. Decorative, and it never moves.
pub fn braille_bruh(size_px: f32) -> impl IntoElement {
    braille_face(size_px, &FaceView::default())
}

/// The Bruh drawn as `view` says: its style, plus a ring when it has one.
pub fn braille_face(size_px: f32, view: &FaceView) -> impl IntoElement {
    let style = view.style;
    let ring = view.ring_rgb;
    let dots = bruh_dots_styled(size_px, &style);
    canvas(
        |_bounds, _window, _cx| {},
        move |bounds, (), window, _cx| {
            for dot in &dots {
                let r = dot.radius;
                let color: Hsla = rgb(dot.rgb_in(&style)).into();
                let origin = point(
                    bounds.origin.x + px(dot.x - r),
                    bounds.origin.y + px(dot.y - r),
                );
                window.paint_quad(
                    fill(Bounds::new(origin, size(px(r * 2.0), px(r * 2.0))), color)
                        .corner_radii(px(r)),
                );
            }
            if let Some(ring_rgb) = ring {
                let radius = size_px * RING_RADIUS;
                let centre = bounds.center();
                let ring_bounds = Bounds::new(
                    point(centre.x - px(radius), centre.y - px(radius)),
                    size(px(radius * 2.0), px(radius * 2.0)),
                );
                window.paint_quad(quad(
                    ring_bounds,
                    px(radius),
                    transparent_black(),
                    px(RING_STROKE_PX),
                    Hsla::from(rgb(ring_rgb)),
                    BorderStyle::Solid,
                ));
            }
        },
    )
    .size(px(size_px))
    .flex_none()
}
