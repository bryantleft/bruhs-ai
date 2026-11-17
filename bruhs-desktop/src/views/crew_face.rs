//! A bruh's face on desktop: the dithered Bruh drawn for its look and status.
//! A working bruh sweeps its gaze, unless the system asks for reduced motion.

use std::time::Duration;

use bruhs_models::crew::{face_view, BruhLook, BruhStatus, FaceView, SWEEP_PERIOD_MS};
use bruhs_models::num::{ToFloatLossy, ToIntSaturating};
use gpui::prelude::*;
use gpui::{div, Animation, AnimationExt, AnyElement, App, SharedString};

use super::braille::braille_face;

/// The face at `progress` (0 to 1) through one sweep.
pub(crate) fn face_at(
    look: &BruhLook,
    status: BruhStatus,
    progress: f32,
    reduced: bool,
) -> FaceView {
    let now_ms = (progress * SWEEP_PERIOD_MS.to_f32_lossy())
        .round()
        .to_i64_saturating();
    face_view(look, status, now_ms, reduced)
}

/// A bruh's face `size_px` square. `key` names it for selectors and animation
/// state; it must be unique among faces on screen.
#[expect(dead_code, reason = "first caller is the crew rail, in a later task")]
pub fn bruh_face(
    key: SharedString,
    size_px: f32,
    look: BruhLook,
    status: BruhStatus,
    cx: &App,
) -> AnyElement {
    let reduced = crate::motion::reduce_motion(cx);
    let face_key = key.clone();
    let still = face_at(&look, status, 0.0, reduced);
    let face = div()
        .debug_selector(move || format!("desktop-bruh-face-{face_key}"))
        .flex_none();
    if !still.animates {
        return face.child(braille_face(size_px, &still)).into_any_element();
    }
    let sweep_key = key.clone();
    face.child(
        div()
            .debug_selector(move || format!("desktop-bruh-face-sweep-{sweep_key}"))
            .flex_none()
            .with_animation(
                SharedString::from(format!("desktop-bruh-face-sweep-{key}")),
                Animation::new(Duration::from_millis(
                    u64::try_from(SWEEP_PERIOD_MS).unwrap_or(2400),
                ))
                .repeat(),
                move |frame, progress| {
                    frame.child(braille_face(
                        size_px,
                        &face_at(&look, status, progress, false),
                    ))
                },
            ),
    )
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animation_progress_maps_to_one_sweep() {
        let look = BruhLook::default();
        assert!(
            (face_at(&look, BruhStatus::Working, 0.25, false)
                .style
                .look_x
                - 0.6)
                .abs()
                < 1e-4
        );
        assert_eq!(
            face_at(&look, BruhStatus::Working, 0.25, true).style.look_x,
            0.0
        );
    }
}
