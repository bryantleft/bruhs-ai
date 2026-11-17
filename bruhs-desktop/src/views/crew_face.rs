//! A bruh's face on desktop: the dithered Bruh drawn for its look and status.
//! A working bruh sweeps its gaze, unless the system asks for reduced motion.
//!
//! The sweep steps on one app-wide clock at about 12 frames a second
//! ([`FACE_STEP_MS`]). A working face that is painted inside the visible
//! part of its window asks the clock for its next step; the clock notifies
//! only the views that asked, and only when the step changes. A face scrolled
//! out of view, folded away or on a hidden page asks for nothing, so it
//! costs nothing until it shows again.

use std::time::{Duration, Instant};

use bruhs_models::crew::{
    face_view, BruhLook, BruhStatus, FaceView, SWEEP_FRAME_MS, SWEEP_PERIOD_MS,
};
use bruhs_models::num::{ToFloatLossy, ToIntSaturating};
use gpui::prelude::*;
use gpui::{canvas, div, AnyElement, App, EntityId, Global, SharedString};

use super::braille::braille_face;

/// One step of the sweep: 80 ms, so 12.5 frames a second.
pub(crate) const FACE_STEP_MS: u64 = SWEEP_FRAME_MS;
/// Steps in one sweep.
const SWEEP_STEPS: u64 = SWEEP_PERIOD_MS.unsigned_abs() / FACE_STEP_MS;

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

/// The step `elapsed` falls on, 0 to `SWEEP_STEPS - 1`.
fn step_at(elapsed: Duration) -> u64 {
    let millis = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    (millis / FACE_STEP_MS) % SWEEP_STEPS.max(1)
}

/// How far through the sweep `step` is, 0 to 1.
fn progress_of(step: u64) -> f32 {
    step.to_f32_lossy() / SWEEP_STEPS.max(1).to_f32_lossy()
}

/// The sweep's clock: when it started, the step last drawn, and the views
/// that painted a visible working face since then.
struct FaceClock {
    started: Instant,
    step: u64,
    waiting: Vec<EntityId>,
    ticking: bool,
}

impl Global for FaceClock {}

impl FaceClock {
    /// The step now, on the executor's clock (which tests can move).
    fn step_now(cx: &App) -> u64 {
        cx.try_global::<FaceClock>()
            .map_or(0, |clock| clock.step_at_now(cx))
    }

    fn step_at_now(&self, cx: &App) -> u64 {
        step_at(
            cx.background_executor()
                .now()
                .saturating_duration_since(self.started),
        )
    }

    /// `view` painted a visible working face: redraw it at the next step.
    fn wake(view: EntityId, cx: &mut App) {
        if cx.try_global::<FaceClock>().is_none() {
            let started = cx.background_executor().now();
            cx.set_global(FaceClock {
                started,
                step: 0,
                waiting: Vec::new(),
                ticking: false,
            });
        }
        let clock = cx.global_mut::<FaceClock>();
        if !clock.waiting.contains(&view) {
            clock.waiting.push(view);
        }
        if clock.ticking {
            return;
        }
        clock.ticking = true;
        cx.spawn(async move |cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(FACE_STEP_MS))
                .await;
            let go_on = cx.update(|cx| {
                let step = FaceClock::step_now(cx);
                let clock = cx.global_mut::<FaceClock>();
                if step == clock.step {
                    return true;
                }
                clock.step = step;
                let views = std::mem::take(&mut clock.waiting);
                if views.is_empty() {
                    // Nothing visible sweeps: stop until a face asks again.
                    clock.ticking = false;
                    return false;
                }
                for view in views {
                    cx.notify(view);
                }
                true
            });
            if !go_on {
                break;
            }
        })
        .detach();
    }
}

/// Whether the clock is running, for tests.
#[cfg(test)]
pub(crate) fn sweep_ticking(cx: &App) -> bool {
    cx.try_global::<FaceClock>()
        .is_some_and(|clock| clock.ticking)
}

/// A bruh's face `size_px` square. `key` names it for selectors; it must be
/// unique among faces on screen.
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
    let now = face_at(&look, status, progress_of(FaceClock::step_now(cx)), false);
    face.child(
        div()
            .debug_selector(move || format!("desktop-bruh-face-sweep-{key}"))
            .relative()
            .flex_none()
            .child(braille_face(size_px, &now))
            .child(
                canvas(
                    |bounds, window, cx| {
                        // Only a face inside the visible part of the window
                        // asks for its next step.
                        if window.content_mask().bounds.intersects(&bounds) {
                            FaceClock::wake(window.current_view(), cx);
                        }
                    },
                    |_, (), _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            ),
    )
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{px, size, Context, TestAppContext, Window};

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

    #[test]
    fn the_sweep_steps_at_about_twelve_frames_a_second() {
        assert_eq!((SWEEP_STEPS, FACE_STEP_MS), (30, 80));
        assert_eq!(step_at(Duration::from_millis(79)), 0);
        assert_eq!(step_at(Duration::from_millis(80)), 1);
        assert_eq!(step_at(Duration::from_millis(2400)), 0);
        assert!((progress_of(15) - 0.5).abs() < 1e-6);
    }

    /// A view with one working face, `offset` pixels down a clipped box
    /// 100 pixels tall, counting its renders.
    struct Holder {
        offset: f32,
        renders: usize,
    }

    impl Render for Holder {
        fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
            self.renders += 1;
            div().size(px(200.0)).child(
                div()
                    .id("clip")
                    .h(px(100.0))
                    .overflow_hidden()
                    .child(div().h(px(self.offset)))
                    .child(bruh_face(
                        "test".into(),
                        20.0,
                        BruhLook::default(),
                        BruhStatus::Working,
                        cx,
                    )),
            )
        }
    }

    fn holder(
        cx: &mut TestAppContext,
        offset: f32,
        reduced: bool,
    ) -> (gpui::Entity<Holder>, &mut gpui::VisualTestContext) {
        cx.update(|cx| crate::motion::set_reduced_motion(reduced, cx));
        let (view, cx) = cx.add_window_view(|_, _| Holder { offset, renders: 0 });
        cx.simulate_resize(size(px(400.0), px(400.0)));
        cx.run_until_parked();
        (view, cx)
    }

    fn renders_over(
        view: &gpui::Entity<Holder>,
        cx: &mut gpui::VisualTestContext,
        steps: u64,
    ) -> usize {
        let before = view.read_with(cx, |view, _| view.renders);
        for _ in 0..steps {
            cx.executor()
                .advance_clock(Duration::from_millis(FACE_STEP_MS));
            cx.run_until_parked();
        }
        view.read_with(cx, |view, _| view.renders) - before
    }

    #[gpui::test]
    fn a_visible_working_face_redraws_its_view_once_a_step(cx: &mut TestAppContext) {
        let (view, cx) = holder(cx, 0.0, false);
        cx.update(|_, cx| assert!(sweep_ticking(cx)));
        // One redraw per 80 ms step, not one per display frame.
        assert_eq!(renders_over(&view, cx, 5), 5);
        // Half steps: a tick that lands on the same step draws nothing.
        let before = view.read_with(cx, |view, _| view.renders);
        for _ in 0..4 {
            cx.executor()
                .advance_clock(Duration::from_millis(FACE_STEP_MS / 2));
            cx.run_until_parked();
        }
        let drawn = view.read_with(cx, |view, _| view.renders) - before;
        assert!(drawn <= 2, "{drawn} redraws in two steps");
    }

    #[gpui::test]
    fn a_face_out_of_view_never_wakes_the_clock(cx: &mut TestAppContext) {
        let (view, cx) = holder(cx, 300.0, false);
        cx.update(|_, cx| assert!(!sweep_ticking(cx)));
        assert_eq!(renders_over(&view, cx, 3), 0);
    }

    #[gpui::test]
    fn reduced_motion_holds_the_face_still_and_never_ticks(cx: &mut TestAppContext) {
        let (view, cx) = holder(cx, 0.0, true);
        assert!(cx.debug_bounds("desktop-bruh-face-test").is_some());
        assert!(cx.debug_bounds("desktop-bruh-face-sweep-test").is_none());
        cx.update(|_, cx| assert!(!sweep_ticking(cx)));
        assert_eq!(renders_over(&view, cx, 3), 0);
    }
}
