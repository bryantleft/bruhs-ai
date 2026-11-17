//! Desktop twin of web `ui::braille::BrailleBruh`: the Bruh drawn in
//! a halftone of dots (Bruhs AGENTS.md section 19). The dots come from
//! `bruhs_models::logic::dither`, so both apps place the same ones.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use bruhs_models::crew::{Eyes, FaceStyle, FaceView, RING_RADIUS, RING_STROKE_PX};
use bruhs_models::logic::dither::{bruh_dots_styled, Dot};
use gpui::{
    canvas, fill, point, quad, rgb, size, transparent_black, BorderStyle, Bounds, Hsla,
    IntoElement, Styled,
};
use gpui_component::zoom::px;

/// What decides where a face's dots sit: its size, gaze and eyes. Colours
/// are applied when the dots are painted.
type DotsKey = (u32, u32, u32, bool);

/// Most dot layouts kept. A sweep has 30 steps, and faces come in a few sizes.
const DOTS_CACHE_MAX: usize = 512;

thread_local! {
    static DOTS: RefCell<HashMap<DotsKey, Rc<Vec<Dot>>>> = RefCell::new(HashMap::new());
}

/// The dots for a face, computed once per size, gaze and eyes.
pub(crate) fn cached_dots(size_px: f32, style: &FaceStyle) -> Rc<Vec<Dot>> {
    let key = (
        size_px.to_bits(),
        style.look_x.to_bits(),
        style.look_y.to_bits(),
        style.eyes == Eyes::Closed,
    );
    DOTS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(dots) = cache.get(&key) {
            return Rc::clone(dots);
        }
        if cache.len() >= DOTS_CACHE_MAX {
            cache.clear();
        }
        let dots = Rc::new(bruh_dots_styled(size_px, style));
        cache.insert(key, Rc::clone(&dots));
        dots
    })
}

/// A static, dithered Bruh `size_px` square. Decorative, and it never moves.
pub fn braille_bruh(size_px: f32) -> impl IntoElement {
    braille_face(size_px, &FaceView::default())
}

/// The Bruh drawn as `view` says: its style, plus a ring when it has one.
pub fn braille_face(size_px: f32, view: &FaceView) -> impl IntoElement {
    let style = view.style;
    let ring = view.ring_rgb;
    let dots = cached_dots(size_px, &style);
    canvas(
        |_bounds, _window, _cx| {},
        move |bounds, (), window, _cx| {
            for dot in dots.iter() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dots_are_computed_once_per_size_gaze_and_eyes() {
        let style = FaceStyle {
            look_x: 0.3,
            ..FaceStyle::default()
        };
        let first = cached_dots(20.0, &style);
        // A colour change reuses the layout.
        let recoloured = FaceStyle {
            face_rgb: 0x12_34_56,
            ..style
        };
        assert!(Rc::ptr_eq(&first, &cached_dots(20.0, &recoloured)));
        assert_eq!(*first, bruh_dots_styled(20.0, &style));
        let turned = FaceStyle {
            look_x: -0.3,
            ..style
        };
        assert!(!Rc::ptr_eq(&first, &cached_dots(20.0, &turned)));
        assert!(!Rc::ptr_eq(&first, &cached_dots(48.0, &style)));
    }
}
